import express from 'express';
import http from 'http';
import path from 'path';
import chalk from 'chalk';
import fs from 'fs';
import {WebSocketServer} from 'ws';
import {Repo} from '@automerge/automerge-repo';
import {NodeWSServerAdapter as WebSocketServerAdapter} from '@automerge/automerge-repo-network-websocket';
import {NodeFSStorageAdapter} from '@automerge/automerge-repo-storage-nodefs';
import os from 'os';
import {createProxyMiddleware} from 'http-proxy-middleware';
import cors from 'cors';

export interface ServerOptions {
  port?: number;
  mode: 'development' | 'production';
  distPath: string | undefined; // Path to the built frontend files
  verbose?: boolean;
  storageDir?: string; // Directory for Automerge storage
  apiProxy?:
    | {
        target: string; // The target URL to proxy to (e.g., 'http://localhost:3001')
        ws?: boolean; // Whether to proxy WebSocket connections
      }
    | undefined;
}

export class TonkRepoServer {
  private app: express.Application;
  private server: http.Server;
  private wss: WebSocketServer;
  private options: ServerOptions;
  private repo: Repo;
  private readyResolvers: ((value: any) => void)[] = [];
  private isReady = false;

  constructor(options: ServerOptions) {
    this.options = {
      port: options.port || (options.mode === 'development' ? 4080 : 8080),
      mode: options.mode,
      distPath: options.distPath,
      verbose: options.verbose ?? true,
      storageDir: options.storageDir || 'automerge-sync-server-data',
      apiProxy: options.apiProxy,
    };

    // Create Express app and HTTP server
    this.app = express();
    this.server = http.createServer(this.app);

    // Create WebSocket server
    this.wss = new WebSocketServer({
      noServer: true,
    });

    // Ensure storage directory exists
    if (!fs.existsSync(this.options.storageDir!)) {
      fs.mkdirSync(this.options.storageDir!);
    }

    const hostname = os.hostname();

    // Initialize Automerge Repo
    const config = {
      network: [new WebSocketServerAdapter(this.wss as any)], // Type assertion needed due to ws types mismatch
      storage: new NodeFSStorageAdapter(this.options.storageDir!),
      __peerId: `storage-server-${hostname}`,
      // Since this is a server, we don't share generously — meaning we only sync documents they already
      // know about and can ask for by ID.
      sharePolicy: async () => false,
    };

    this.repo = new Repo(config);

    this.repo.on('document', async payload => {
      console.log('received payload data:', await payload.handle.doc());
      console.log('received heads:', payload.handle.heads());
      console.log('received url:', payload.handle.url);
    });

    // Set up metrics and logging
    this.setupExpressMiddleware();
    this.setupWebSocketHandlers();
  }

  private log(color: 'green' | 'red' | 'blue' | 'yellow', message: string) {
    if (this.options.verbose) {
      console.log(chalk[color](message));
    }
  }

  private setupWebSocketHandlers() {
    // Handle WebSocket upgrade
    this.server.on('upgrade', (request, socket, head) => {
      this.wss.handleUpgrade(request, socket, head, socket => {
        this.wss.emit('connection', socket, request);
      });
    });
  }

  private setupExpressMiddleware() {
    // Basic routes
    this.app.get('/ping', (_req, res) => {
      res.send('pong');
    });

    this.app.get('/', (_req, res) => {
      res.send(`👍 Tonk Automerge Server is running`);
    });

    // If apiProxy is configured, use proxy middleware
    if (this.options.apiProxy) {
      this.app.use('/api', cors());
      this.app.use(
        '/api',
        createProxyMiddleware({
          target: this.options.apiProxy?.target,
          changeOrigin: true,
        }),
      );
    }

    // In production mode, serve static files and handle client-side routing
    if (this.options.mode === 'production' && this.options.distPath) {
      // Handle WASM files with correct MIME type
      this.app.get('*.wasm', (_req, res, next) => {
        res.set('Content-Type', 'application/wasm');
        next();
      });

      // Static file serving after specific routes
      this.app.use(express.static(this.options.distPath));

      // Client-side routing - only match routes that don't start with /api or other special routes
      this.app.get(/^(?!\/api|\/ping|\/prometheus_metrics).*$/, (_req, res) => {
        res.sendFile(path.join(this.options.distPath!, 'index.html'));
      });
    }
  }

  public async ready() {
    if (this.isReady) {
      return true;
    }

    return new Promise(resolve => {
      this.readyResolvers.push(resolve);
    });
  }

  public start(): Promise<void> {
    console.log(this.repo);
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

        this.isReady = true;
        this.readyResolvers.forEach(resolve => resolve(true));
        resolve();
      });
    });
  }

  public stop(): Promise<void> {
    return new Promise((resolve, reject) => {
      this.wss.close(err => {
        if (err) {
          this.log('red', `Error closing WebSocket server: ${err.message}`);
          reject(err);
        } else {
          this.server.close(err => {
            if (err) {
              reject(err);
            } else {
              resolve();
            }
          });
        }
      });
    });
  }

  // Helper method to update service worker to cache WASM files in production
  private async updateWasmCache(): Promise<void> {
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
}

// Convenience function to create and start a server
export async function createServer(
  options: ServerOptions,
): Promise<TonkRepoServer> {
  const server = new TonkRepoServer(options);
  await server.start();
  return server;
}
