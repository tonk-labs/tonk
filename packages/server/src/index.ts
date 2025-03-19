import {WebSocketServer, WebSocket} from 'ws';
import express from 'express';
import http from 'http';
import path from 'path';
import chalk from 'chalk';
import fs from 'fs';
import {StorageMiddleware, StorageMiddlewareOptions} from './storageMiddleware';

export interface ServerOptions {
  port?: number;
  mode: 'development' | 'production';
  distPath: string | undefined; // Path to the built frontend files
  verbose?: boolean;
  storage?: StorageMiddlewareOptions | undefined;
}

export class TonkServer {
  private app: express.Application;
  private server: http.Server;
  private wss: WebSocketServer;
  private connections: Set<WebSocket> = new Set();
  private options: ServerOptions;
  private storageMiddleware: StorageMiddleware | null = null;

  constructor(options: ServerOptions) {
    this.options = {
      port: options.port || (options.mode === 'development' ? 4080 : 8080),
      mode: options.mode,
      distPath: options.distPath,
      verbose: options.verbose ?? true,
      storage: options.storage,
    };

    // Create Express app and HTTP server
    this.app = express();
    this.server = http.createServer(this.app);

    // Create WebSocket server
    this.wss = new WebSocketServer({
      server: this.server,
      path: '/sync',
    });

    // Initialize storage middleware if configured
    if (this.options.storage) {
      this.storageMiddleware = new StorageMiddleware(
        this.options.storage,
        this.log.bind(this),
        this.options.verbose,
      );
    }

    this.setupWebSocketHandlers();
    this.setupExpressMiddleware();
  }

  private setupWebSocketHandlers() {
    // Handle WebSocket connections
    this.wss.on('connection', (ws: WebSocket) => {
      this.log('green', `Client connected`);
      this.connections.add(ws);

      // Handle messages from clients
      ws.on('message', (data: Buffer) => {
        this.connections.forEach(client => {
          if (client !== ws && client.readyState === WebSocket.OPEN) {
            client.send(data.toString());
          }
        });

        // Forward to storage middleware if enabled
        if (this.storageMiddleware) {
          this.storageMiddleware.handleMessage(data);
        }
      });

      // Handle client disconnection
      ws.on('close', () => {
        this.log('red', `Client disconnected`);
        this.connections.delete(ws);
      });

      // Handle errors
      ws.on('error', (error: Error) => {
        this.log('red', `WebSocket error: ${error.message}`);
      });
    });
  }

  private setupExpressMiddleware() {
    // In production mode, serve static files and handle client-side routing
    if (this.options.mode === 'production' && this.options.distPath) {
      // Handle WASM files with correct MIME type
      this.app.get('*.wasm', (_req, res, next) => {
        res.set('Content-Type', 'application/wasm');
        next();
      });

      this.app.use(express.static(this.options.distPath));

      // Send all requests to index.html for client-side routing
      this.app.get('*', (_req, res) => {
        res.sendFile(path.join(this.options.distPath!, 'index.html'));
      });
    }
  }

  // Helper method to update service worker to cache WASM files in production
  public async updateWasmCache(): Promise<void> {
    if (this.options.mode !== 'production' || !this.options.distPath) {
      return;
    }

    try {
      const distDir = this.options.distPath;
      const serviceWorkerPath = path.join(distDir, 'service-worker.js');

      // Check if service worker exists
      if (!fs.existsSync(serviceWorkerPath)) {
        this.log(
          'yellow',
          'No service worker found. Skipping WASM cache update.',
        );
        return;
      }

      // Find the WASM file
      const files = fs.readdirSync(distDir);
      const wasmFile = files.find(file => file.endsWith('.wasm'));

      if (!wasmFile) {
        this.log('yellow', 'No WASM file found. Skipping cache update.');
        return;
      }

      this.log('blue', `Found WASM file: ${wasmFile}`);

      // Update the service worker
      let swContent = fs.readFileSync(serviceWorkerPath, 'utf8');
      const wasmPattern = /["']([^"']+\.wasm)["']/g;
      const wasmMatches = [...swContent.matchAll(wasmPattern)];

      if (wasmMatches.length === 0) {
        this.log('yellow', 'No WASM file reference found in service worker');
        return;
      }

      // Replace all occurrences of WASM files with the current one
      for (const match of wasmMatches) {
        const oldWasmPath = match[1];
        // If the path starts with a slash, we need to add it to our replacement
        const prefix = oldWasmPath.startsWith('/') ? '/' : '';
        swContent = swContent.replace(oldWasmPath, `${prefix}${wasmFile}`);
      }

      // Write the updated service worker back to disk
      fs.writeFileSync(serviceWorkerPath, swContent);
      this.log(
        'green',
        `Service worker updated to cache WASM file: ${wasmFile}`,
      );
    } catch (error) {
      this.log('red', `Error updating WASM cache: ${error}`);
    }
  }

  private log(color: 'green' | 'red' | 'blue' | 'yellow', message: string) {
    if (this.options.verbose) {
      console.log(chalk[color](message));
    }
  }

  public start(): Promise<void> {
    return new Promise(async resolve => {
      if (this.options.mode === 'production' && this.options.distPath) {
        await this.updateWasmCache();
      }

      this.server.listen(this.options.port, () => {
        this.log('green', `Server running on port ${this.options.port}`);

        if (this.options.mode === 'production') {
          this.log(
            'blue',
            `Open http://localhost:${this.options.port} to view your app`,
          );
          this.log(
            'blue',
            "On your phone, connect to this server using your computer's local IP address",
          );
          this.log(
            'blue',
            `Use pinggy to expose this server: ssh -p 443 -R0:localhost:${this.options.port} a.pinggy.io`,
          );
        } else {
          this.log(
            'blue',
            `Development sync server running on port ${this.options.port}`,
          );
        }

        resolve();
      });
    });
  }

  public stop(): Promise<void> {
    return new Promise(async (resolve, reject) => {
      // Shut down storage middleware if enabled
      if (this.storageMiddleware) {
        try {
          await this.storageMiddleware.shutdown();
        } catch (error) {
          this.log(
            'red',
            `Error shutting down storage middleware: ${(error as Error).message}`,
          );
        }
      }

      this.connections.forEach(conn => {
        conn.terminate();
      });
      this.connections.clear();

      this.server.close(err => {
        if (err) {
          reject(err);
        } else {
          resolve();
        }
      });
    });
  }
}

// Convenience function to create and start a server
export async function createServer(
  options: ServerOptions,
): Promise<TonkServer> {
  const server = new TonkServer(options);
  await server.start();
  return server;
}
