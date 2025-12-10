import { spawn } from 'child_process';
import { readFile, rm } from 'fs/promises';
import * as net from 'net';
import * as path from 'path';
import { fileURLToPath } from 'url';
import type { ServerInstance } from '../test-ui/types';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

async function getRelayBinaryPath(): Promise<string> {
  const binaryPathFile = path.resolve(__dirname, '../../.relay-binary');
  try {
    const binaryPath = await readFile(binaryPathFile, 'utf-8');
    return binaryPath.trim();
  } catch (error) {
    throw new Error(
      'Relay binary path not found. Make sure global setup has run successfully.\n' +
        'To build the basic relay: cd packages/relay && cargo build\n' +
        'For proprietary relay: nix develop .#withKnot --override-input knot path:../knot'
    );
  }
}

export class ServerManager {
  private servers: Map<string, ServerInstance> = new Map();
  private usedPorts: Set<number> = new Set();

  /**
   * Find an available port in the specified range
   */
  private async findFreePort(
    startPort: number,
    endPort: number
  ): Promise<number> {
    for (let port = startPort; port <= endPort; port++) {
      if (this.usedPorts.has(port)) continue;

      const isAvailable = await this.isPortAvailable(port);
      if (isAvailable) {
        this.usedPorts.add(port);
        return port;
      }
    }
    throw new Error(`No free port found between ${startPort} and ${endPort}`);
  }

  /**
   * Check if a port is available
   */
  private isPortAvailable(port: number): Promise<boolean> {
    return new Promise(resolve => {
      const server = net.createServer();

      server.once('error', () => {
        resolve(false);
      });

      server.once('listening', () => {
        server.close();
        resolve(true);
      });

      server.listen(port, '127.0.0.1');
    });
  }

  /**
   * Wait for server to be ready by polling the HTTP endpoint
   */
  private async waitForServerReady(
    port: number,
    timeout = 30000
  ): Promise<void> {
    const startTime = Date.now();

    while (Date.now() - startTime < timeout) {
      try {
        const response = await fetch(`http://localhost:${port}/`);
        if (response.ok) {
          console.log(`Server on port ${port} is ready`);
          return;
        }
      } catch (error) {
        // Server not ready yet, continue waiting
      }

      await new Promise(resolve => setTimeout(resolve, 500));
    }

    throw new Error(
      `Server on port ${port} did not become ready within ${timeout}ms`
    );
  }

  /**
   * Start a new server instance for a test
   */
  async startServer(testId: string): Promise<ServerInstance> {
    // Find a free port
    const port = await this.findFreePort(8100, 8999);

    const relayBinaryPath = await getRelayBinaryPath();

    const blankTonkPath = path.resolve(
      __dirname,
      '../../../examples/latergram/latergram.tonk'
    );

    // Create unique storage directory for this test
    const storageDir = path.resolve(
      __dirname,
      `../../../packages/relay/test-storage/${testId}`
    );

    console.log(`Starting server for test ${testId} on port ${port}`);
    console.log(`Relay binary: ${relayBinaryPath}`);
    console.log(`Bundle file: ${blankTonkPath}`);
    console.log(`Storage dir: ${storageDir}`);

    // Spawn the server process using the pre-compiled binary
    const serverProcess = spawn(
      relayBinaryPath,
      [port.toString(), blankTonkPath, storageDir],
      {
        stdio: ['ignore', 'pipe', 'pipe'],
        detached: false,
        env: {
          ...process.env,
          S3_BUCKET_NAME: 'host-web-bundle-storage-test',
          RUST_LOG: 'info',
        },
      }
    );

    // Log server output for debugging
    serverProcess.stdout?.on('data', (data: Buffer) => {
      console.log(`[Server ${port}]: ${data.toString()}`);
    });

    serverProcess.stderr?.on('data', (data: Buffer) => {
      console.error(`[Server ${port} Error]: ${data.toString()}`);
    });

    // Handle server exit
    serverProcess.on(
      'exit',
      (code: number | null, signal: NodeJS.Signals | null) => {
        console.log(
          `Server on port ${port} exited with code ${code}, signal ${signal}`
        );
        this.usedPorts.delete(port);
        this.servers.delete(testId);
      }
    );

    // Store server instance
    const instance: ServerInstance = {
      process: serverProcess,
      port,
      wsUrl: `ws://localhost:${port}`,
      manifestUrl: `http://localhost:${port}/api/blank-tonk`,
      testId,
    };

    this.servers.set(testId, instance);

    // Wait for server to be ready
    try {
      await this.waitForServerReady(port);
    } catch (error) {
      // If server didn't start, clean up
      this.stopServer(testId);
      throw error;
    }

    return instance;
  }

  /**
   * Stop a server instance
   */
  async stopServer(testId: string): Promise<void> {
    const server = this.servers.get(testId);
    if (!server) {
      console.log(`No server found for test ${testId}`);
      return;
    }

    console.log(`Stopping server for test ${testId} on port ${server.port}`);

    // Kill the process
    if (server.process && typeof server.process.kill === 'function') {
      server.process.kill('SIGTERM');

      // Give it time to shut down gracefully
      await new Promise(resolve => setTimeout(resolve, 1000));

      // Force kill if still running
      try {
        server.process.kill('SIGKILL');
      } catch (error) {
        // Process might already be dead
      }
    }

    // Clean up storage directory
    const storageDir = path.resolve(
      __dirname,
      `../../../packages/relay/test-storage/${testId}`
    );
    try {
      await rm(storageDir, { recursive: true, force: true });
      console.log(`Cleaned up storage directory: ${storageDir}`);
    } catch (error) {
      console.error(`Failed to clean up storage directory: ${error}`);
    }

    // Clean up tracking
    this.usedPorts.delete(server.port);
    this.servers.delete(testId);
  }

  /**
   * Stop all server instances
   */
  async cleanupAll(): Promise<void> {
    console.log(`Cleaning up ${this.servers.size} server(s)`);

    const stopPromises: Promise<void>[] = [];
    for (const [testId] of Array.from(this.servers.keys())) {
      stopPromises.push(this.stopServer(testId));
    }

    await Promise.all(stopPromises);

    this.servers.clear();
    this.usedPorts.clear();
  }

  /**
   * Get server info for a test
   */
  getServer(testId: string): ServerInstance | undefined {
    return this.servers.get(testId);
  }

  /**
   * Get all active servers
   */
  getActiveServers(): Map<string, ServerInstance> {
    return new Map(this.servers);
  }
}

// Export singleton instance
export const serverManager = new ServerManager();

// Cleanup on process exit
process.on('exit', () => {
  serverManager.cleanupAll().catch(console.error);
});

process.on('SIGINT', () => {
  serverManager
    .cleanupAll()
    .then(() => process.exit(0))
    .catch(console.error);
});

process.on('SIGTERM', () => {
  serverManager
    .cleanupAll()
    .then(() => process.exit(0))
    .catch(console.error);
});
