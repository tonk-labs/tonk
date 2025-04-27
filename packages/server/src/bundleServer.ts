import express from 'express';
import http from 'http';
import path from 'path';
import fs from 'fs';
import {createProxyMiddleware} from 'http-proxy-middleware';
import {BundleServerConfig} from './types.js';
import chalk from 'chalk';

export class BundleServer {
  private app: express.Application;
  private server: http.Server;
  private config: BundleServerConfig;
  private isRunning: boolean = false;
  private startTime?: Date;

  constructor(config: BundleServerConfig) {
    this.config = config;
    this.app = express();
    this.server = http.createServer(this.app);
    this.setupRoutes();
  }

  private log(color: 'green' | 'red' | 'blue' | 'yellow', message: string) {
    if (this.config.verbose !== false) {
      console.log(chalk[color](message));
    }
  }

  private setupRoutes() {
    // Handle WASM files with correct MIME type
    this.app.get('*.wasm', (_req, res, next) => {
      res.set('Content-Type', 'application/wasm');
      next();
    });

    // If the bundle has a services directory, set up a proxy to it
    // TODO this needs to load in services from the bundle and plug that functionality into an express server which will be this proxy target
    // if (this.config.hasServices) {
    //   this.app.use(
    //     '/api',
    //     createProxyMiddleware({
    //       target: `http://localhost:${this.config.port}`,
    //       changeOrigin: true,
    //     }),
    //   );
    // }

    // Serve static files from the bundle
    this.app.use(express.static(this.config.bundlePath));

    // Client-side routing - for SPA, send index.html for all non-api paths
    this.app.get(/^(?!\/api|\/services).*$/, (_req, res) => {
      res.sendFile(path.join(this.config.bundlePath, 'index.html'));
    });

    const wsProxy = createProxyMiddleware({
      target: 'ws://localhost:7777',
      changeOrigin: true,
      ws: true,
      pathFilter: '/sync',
      // @ts-expect-error - These options help with binary data
      bufferData: true,
      // Prevent automatic parsing of messages
      onProxyReqWs: (
        _proxyReq: http.ClientRequest,
        _req: http.IncomingMessage,
        _socket: any,
      ) => {
        // Don't modify the incoming WebSocket requests
      },
      onError: (
        err: Error,
        _req: http.IncomingMessage,
        res: http.ServerResponse,
      ) => {
        this.log('red', `WebSocket proxy error: ${err.message}`);
        if (res.writeHead && !res.headersSent) {
          res.writeHead(500);
          res.end('Proxy error');
        }
      },
      onProxyRes: (proxyRes: http.IncomingMessage) => {
        this.log('blue', `Proxy response status: ${proxyRes.statusCode}`);
      },
    });
    // Proxy /sync requests to localhost:7777
    this.app.use(wsProxy);

    this.server.on('upgrade', wsProxy.upgrade);
  }

  // Helper method to update service worker to cache WASM files
  public async updateWasmCache(): Promise<void> {
    try {
      const bundlePath = this.config.bundlePath;
      const serviceWorkerPath = path.join(bundlePath, 'service-worker.js');

      // Check if service worker exists
      if (!fs.existsSync(serviceWorkerPath)) {
        this.log(
          'yellow',
          'No service worker found. Skipping WASM cache update.',
        );
        return;
      }

      // Find the WASM file
      const files = fs.readdirSync(bundlePath);
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

  public async start(): Promise<void> {
    // Update WASM cache before starting the server
    await this.updateWasmCache();

    return new Promise((resolve, reject) => {
      if (this.isRunning) {
        return resolve();
      }

      this.server
        .listen(this.config.port, () => {
          this.isRunning = true;
          this.startTime = new Date();
          this.log(
            'green',
            `Bundle server for ${this.config.bundleName} started on port ${this.config.port}`,
          );
          resolve();
        })
        .on('error', err => {
          this.log('red', `Failed to start bundle server: ${err.message}`);
          reject(err);
        });
    });
  }

  public stop(): Promise<void> {
    return new Promise(resolve => {
      if (!this.isRunning) {
        return resolve();
      }

      this.server.closeAllConnections();
      this.server.close(() => {
        this.isRunning = false;
        this.log(
          'yellow',
          `Bundle server for ${this.config.bundleName} stopped`,
        );
        resolve();
      });
    });
  }

  public getStatus() {
    return {
      isRunning: this.isRunning,
      port: this.config.port,
      bundleName: this.config.bundleName,
      startTime: this.startTime,
    };
  }
}
