import {createProxyMiddleware} from 'http-proxy-middleware';
import express from 'express';
import http from 'http';
import path from 'path';
import chalk from 'chalk';
import fs from 'fs';
import os from 'os';
import {WebSocketServer} from 'ws';
import {PeerId, Repo, RepoConfig} from '@tonk/automerge-repo';
import {NodeWSServerAdapter} from '@automerge/automerge-repo-network-websocket';
import {NodeFSStorageAdapter} from '@automerge/automerge-repo-storage-nodefs';

export interface ServerOptions {
  port?: number;
  mode: 'development' | 'production';
  distPath?: string; // Path to the built frontend files
  dirPath?: string;
  verbose?: boolean;
  syncInterval?: number; // Optional interval to trigger storage sync in ms
}

export class TonkServer {
  private app: express.Application;
  private socket: WebSocketServer;
  private server: http.Server;
  private readyResolvers: ((value: any) => void)[] = [];
  // @ts-ignore
  private repo: Repo;
  private options: ServerOptions;
  private initialDistPathSet: boolean = false; // Track if distPath was set at construction time
  private serverManagerPingInterval: NodeJS.Timeout | null = null;
  private serverManagerProxyActive: boolean = false;
  private fsSyncTimer: NodeJS.Timeout | null = null;

  constructor(options: ServerOptions) {
    this.options = {
      port: options.port || (options.mode === 'development' ? 4080 : 8080),
      mode: options.mode,
      distPath: options.distPath!,
      dirPath: options.dirPath || 'stores',
      syncInterval: options.syncInterval || 0,
      verbose: options.verbose ?? true,
    };

    if (!fs.existsSync(this.options.dirPath!))
      fs.mkdirSync(this.options.dirPath!);

    const hostname = os.hostname();
    this.socket = new WebSocketServer({noServer: true});

    // Track if distPath was initially set
    this.initialDistPathSet = this.options.distPath !== undefined;

    // Create Express app and HTTP server
    this.app = express();
    this.server = http.createServer(this.app);

    const config: RepoConfig = {
      network: [new NodeWSServerAdapter(this.socket as any)],
      storage: new NodeFSStorageAdapter(this.options.dirPath!),
      peerId: `sync-server-${hostname}` as PeerId,
      sharePolicy: async () => false,
    };
    this.repo = new Repo(config);

    this.setupExpressMiddleware();
  }

  private log(color: 'green' | 'red' | 'blue' | 'yellow', message: string) {
    if (this.options.verbose) {
      console.log(chalk[color](message));
    }
  }

  private setupExpressMiddleware() {
    this.app.get('/ping', (_req, res) => {
      res.send('pong');
    });

    // Add endpoint to toggle distPath value
    this.app.post('/api/toggle-dist-path', express.json(), (req, res) => {
      // Only set distPath if it was not set at construction time
      if (!this.initialDistPathSet) {
        // Use the distPath from the request body
        if (req.body && req.body.distPath !== undefined) {
          this.options.distPath = req.body.distPath;
          this.log('green', `distPath set to: ${this.options.distPath}`);

          // Refresh the static file middleware
          this.refreshStaticFileMiddleware();

          res.json({
            success: true,
            distPath: this.options.distPath,
            message: 'distPath updated successfully',
          });
        } else {
          res.json({
            success: false,
            distPath: this.options.distPath,
            message: 'No distPath provided in request body',
          });
        }
      } else {
        // Don't update if initially set
        res.json({
          success: false,
          distPath: this.options.distPath,
          message: 'Cannot update distPath that was set at construction time',
        });
      }
    });

    // Set up static file middleware for production mode
    this.setupStaticFileMiddleware();
  }

  // In production mode, serve static files and handle client-side routing
  private setupStaticFileMiddleware(): void {
    if (this.options.mode === 'production' && this.options.distPath) {
      // Handle WASM files with correct MIME type
      this.app.get('*.wasm', (_req, res, next) => {
        res.set('Content-Type', 'application/wasm');
        next();
      });

      // Static file serving after specific routes
      this.app.use(express.static(this.options.distPath));

      // Client-side routing - only match routes that don't start with /api or /ping
      this.app.get(/^(?!\/api|\/ping).*$/, (_req, res) => {
        res.sendFile(path.join(this.options.distPath!, 'index.html'));
      });
    }
  }

  // Helper method to refresh the static file middleware when distPath changes
  private refreshStaticFileMiddleware(): void {
    if (this.options.mode === 'production' && this.options.distPath) {
      // Remove any existing static middleware (not directly possible with Express)
      // Instead, we'll set up the routes again in the correct order

      // Re-apply the WASM mime type handler
      this.app._router.stack = this.app._router.stack.filter(
        (layer: any) => !(layer.route && layer.route.path === '*.wasm'),
      );
      this.app.get('*.wasm', (_req, res, next) => {
        res.set('Content-Type', 'application/wasm');
        next();
      });

      // Remove existing static middleware
      this.app._router.stack = this.app._router.stack.filter(
        (layer: any) => layer.name !== 'serveStatic',
      );

      // Re-add static file serving with new path
      this.app.use(express.static(this.options.distPath));

      // Remove existing catchall route for client-side routing
      // Using string representation for the regex comparison
      this.app._router.stack = this.app._router.stack.filter(
        (layer: any) =>
          !(
            layer.route &&
            layer.route.path &&
            layer.route.path.toString() === /^(?!\/api|\/ping).*$/.toString()
          ),
      );

      // Re-add client-side routing handler
      this.app.get(/^(?!\/api|\/ping).*$/, (_req, res) => {
        res.sendFile(path.join(this.options.distPath!, 'index.html'));
      });

      this.log(
        'blue',
        `Static file middleware refreshed with path: ${this.options.distPath}`,
      );
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

  private startServerManagerMonitoring(): void {
    if (this.serverManagerPingInterval) {
      clearInterval(this.serverManagerPingInterval);
      this.serverManagerPingInterval = null;
    }

    this.log('blue', 'Starting server manager monitoring...');

    // Try to ping the server manager every 2 seconds
    this.serverManagerPingInterval = setInterval(async () => {
      try {
        const response = await fetch('http://localhost:6080/ping', {
          method: 'GET',
          headers: {'Content-Type': 'application/json'},
        });

        if (response.ok) {
          // If we get a response and the proxy isn't active, set it up
          if (!this.serverManagerProxyActive) {
            this.setupServerManagerProxy();
          }
        } else {
          // If we get a bad response and the proxy is active, remove it
          if (this.serverManagerProxyActive) {
            this.removeServerManagerProxy();
          }
        }
      } catch (error: any) {
        // If we can't connect and the proxy is active, remove it
        if (this.serverManagerProxyActive) {
          this.log('yellow', `Server manager not responding: ${error.message}`);
          this.removeServerManagerProxy();
        }
      }
    }, 2000);
  }

  private setupServerManagerProxy(): void {
    if (this.serverManagerProxyActive) return;

    this.log('green', 'Server manager found, setting up proxy');

    // Create a proxy middleware to the server manager
    const serverManagerProxy = createProxyMiddleware({
      target: 'http://localhost:6080/api',
      changeOrigin: true,
    });

    // Add the proxy middleware
    this.app.use('/api', serverManagerProxy);
    this.serverManagerProxyActive = true;

    this.log('green', 'Server manager proxy active at /api');
  }

  private removeServerManagerProxy(): void {
    if (!this.serverManagerProxyActive) return;

    this.log('yellow', 'Removing server manager proxy');

    // Remove the proxy middleware
    this.app._router.stack = this.app._router.stack.filter(
      (layer: any) => !layer.route || layer.route.path !== '/api',
    );

    this.serverManagerProxyActive = false;
  }

  private stopServerManagerMonitoring(): void {
    if (this.serverManagerPingInterval) {
      clearInterval(this.serverManagerPingInterval);
      this.serverManagerPingInterval = null;
      this.log('yellow', 'Server manager monitoring stopped');
    }

    this.removeServerManagerProxy();
  }

  public start(): Promise<void> {
    return new Promise(async resolve => {
      if (this.options.mode === 'production' && this.options.distPath) {
        await this.updateWasmCache();
      }
      // Start monitoring for server manager instead of launching it
      this.startServerManagerMonitoring();

      this.server.listen(this.options.port, () => {
        this.log(
          'green',
          `Server running on port ${this.options.port}, update: 6`,
        );

        this.readyResolvers.forEach((resolve: any) => resolve(true));

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

          // Also start monitoring in development mode
          this.startServerManagerMonitoring();
        }

        resolve();
      });

      this.server.on('upgrade', (request, socket, head) => {
        this.socket.handleUpgrade(request, socket, head, socket => {
          this.socket.emit('connection', socket, request);
        });
      });
    });
  }

  public async stop(): Promise<void> {
    if (this.fsSyncTimer) {
      clearInterval(this.fsSyncTimer);
      this.fsSyncTimer = null;
    }

    return new Promise((resolve, reject) => {
      // Stop the server manager monitoring
      this.stopServerManagerMonitoring();

      this.socket.close();
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
