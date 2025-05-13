import express from 'express';
import http from 'http';
import path from 'path';
import fs from 'fs';
import {createProxyMiddleware} from 'http-proxy-middleware';
import {BundleServerConfig} from './types.js';
import chalk from 'chalk';
import {RootNode} from './rootNode.js';
import {ApiService} from './types.js';

export class BundleServer {
  private app: express.Application;
  private server: http.Server;
  private config: BundleServerConfig;
  private isRunning: boolean = false;
  private startTime?: Date;
  private rootNode: RootNode;

  constructor(config: BundleServerConfig) {
    this.config = config;
    this.app = express();
    this.server = http.createServer(this.app);
    this.rootNode = config.rootNode;
    this.setupRoutes();
  }

  private log(color: 'green' | 'red' | 'blue' | 'yellow', message: string) {
    if (this.config.verbose !== false) {
      console.log(chalk[color](message));
    }
  }

  private setupApiProxies() {
    const apiServicesPath = path.join(
      this.config.bundlePath,
      'apiServices.json',
    );

    // Check if apiServices.json exists
    if (!fs.existsSync(apiServicesPath)) {
      this.log(
        'yellow',
        'No apiServices.json found. Skipping API proxy setup.',
      );
      return;
    }

    try {
      // Read and parse the apiServices.json file
      const apiServicesContent = fs.readFileSync(apiServicesPath, 'utf8');
      const apiServices: ApiService[] = JSON.parse(apiServicesContent);

      if (!Array.isArray(apiServices) || apiServices.length === 0) {
        this.log(
          'yellow',
          'apiServices.json is empty or not an array. Skipping API proxy setup.',
        );
        return;
      }

      // Set up a proxy for each API service
      for (const service of apiServices) {
        const {
          prefix,
          baseUrl,
          requiresAuth,
          authType,
          authHeaderName,
          authEnvVar,
          authQueryParamName,
        } = service;

        if (!prefix || !baseUrl) {
          this.log(
            'yellow',
            `Skipping invalid API service: ${JSON.stringify(service)}`,
          );
          continue;
        }

        // For query parameter authentication, we need to handle it differently
        if (requiresAuth && authType === 'query' && authQueryParamName) {
          // Create a router to handle query parameter authentication
          const router = express.Router();
          
          // Middleware to add the query parameter to all requests
          router.use((req, _res, next) => {
            let authValue = authEnvVar || '';
            
            // If authEnvVar is specified, try to get it from environment variables
            if (authEnvVar && authEnvVar.startsWith('$')) {
              const envVarName = authEnvVar.substring(1);
              authValue = process.env[envVarName] || authEnvVar;
            }
            
            // Add the query parameter to the URL
            const url = new URL(req.url, 'http://localhost');
            url.searchParams.set(authQueryParamName, authValue);
            req.url = url.pathname + url.search;
            
            if (this.config.verbose) {
              this.log(
                'blue',
                `Added query param ${authQueryParamName} to request: ${req.url}`,
              );
            }
            
            next();
          });
          
          // Mount the router at the API path
          const proxyPath = `/api/${prefix}`;
          this.log('blue', `Setting up API proxy with query auth: ${proxyPath} -> ${baseUrl}`);
          
          this.app.use(
            proxyPath,
            router,
            createProxyMiddleware({
              target: baseUrl,
              changeOrigin: true,
              pathRewrite: {
                [`^/api/${prefix}`]: '', // Remove the /api/prefix part when forwarding
              },
              on: {
                proxyReq: (_proxyReq, req, _res) => {
                  // Log the proxied request in verbose mode
                  if (this.config.verbose) {
                    this.log(
                      'blue',
                      `Proxying request: ${req.method} ${req.url} -> ${baseUrl}`,
                    );
                  }
                },
                error: (err, _req, res) => {
                  this.log(
                    'red',
                    `API proxy error for ${prefix}: ${err.message}`,
                  );
                  res.end(
                    JSON.stringify({error: 'Proxy error', message: err.message}),
                  );
                },
              },
            }),
          );
        } else {
          // Standard header-based authentication
          const proxyPath = `/api/${prefix}`;
          this.log('blue', `Setting up API proxy: ${proxyPath} -> ${baseUrl}`);

          this.app.use(
            proxyPath,
            createProxyMiddleware({
              target: baseUrl,
              changeOrigin: true,
              pathRewrite: {
                [`^/api/${prefix}`]: '', // Remove the /api/prefix part when forwarding
              },
              on: {
                proxyReq: (proxyReq, req, _res) => {
                  // Add authentication if required
                  if (requiresAuth && authType) {
                    let authValue = authEnvVar;

                    // If authEnvVar is specified, try to get it from environment variables
                    if (authEnvVar && authEnvVar.startsWith('$')) {
                      const envVarName = authEnvVar.substring(1);
                      authValue = process.env[envVarName] || authEnvVar;
                    }

                    if (authType === 'apikey' && authHeaderName) {
                      proxyReq.setHeader(authHeaderName, authValue || '');
                    } else if (authType === 'bearer') {
                      proxyReq.setHeader(
                        'Authorization',
                        `Bearer ${authValue || ''}`,
                      );
                    } else if (authType === 'basic') {
                      proxyReq.setHeader(
                        'Authorization',
                        `Basic ${authValue || ''}`,
                      );
                    }
                  }

                  // Log the proxied request in verbose mode
                  if (this.config.verbose) {
                    this.log(
                      'blue',
                      `Proxying request: ${req.method} ${req.url} -> ${baseUrl}`,
                    );
                  }
                },
                error: (err, _req, res) => {
                  this.log(
                    'red',
                    `API proxy error for ${prefix}: ${err.message}`,
                  );
                  res.end(
                    JSON.stringify({error: 'Proxy error', message: err.message}),
                  );
                },
              },
            }),
          );
        }
      }

      this.log(
        'green',
        `API proxies set up for ${apiServices.length} services`,
      );
    } catch (error) {
      this.log('red', `Error setting up API proxies: ${error}`);
    }
  }

  private setupRoutes() {
    // Handle WASM files with correct MIME type
    this.app.get('*.wasm', (_req, res, next) => {
      res.set('Content-Type', 'application/wasm');
      next();
    });

    // Set up API proxies based on apiServices.json if it exists
    this.setupApiProxies();

    // Serve static files from the bundle
    this.app.use(express.static(this.config.bundlePath));

    // Client-side routing - for SPA, send index.html for all non-api paths
    this.app.get(/^(?!\/api|\/services).*$/, (req, res) => {
      if (req.path === '/.well-known/root.json') {
        return res.sendFile(this.rootNode.getRootIdFilePath());
      }
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
