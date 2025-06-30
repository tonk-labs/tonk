import express from 'express';
import http from 'http';
import path from 'path';
import chalk from 'chalk';
import fs from 'fs';
import os from 'os';
import {WebSocketServer} from 'ws';
import {PeerId, Repo, RepoConfig, DocumentId} from '@automerge/automerge-repo';
import {NodeWSServerAdapter} from '@automerge/automerge-repo-network-websocket';
import {NodeFSStorageAdapter} from '@automerge/automerge-repo-storage-nodefs';
import multer from 'multer';
import * as tar from 'tar';
import cors from 'cors';
import {v4 as uuidv4} from 'uuid';
import {createProxyMiddleware} from 'http-proxy-middleware';
import {BundleServer} from './bundleServer.js';
import {BundleServerInfo} from './types.js';
import {RootNode} from './rootNode.js';
import {BundlePersistence} from './bundlePersistence.js';

// A simple mutex implementation for bundle extraction
class Mutex {
  private locked: boolean = false;
  private waitQueue: (() => void)[] = [];

  async acquire(): Promise<void> {
    if (!this.locked) {
      this.locked = true;
      return Promise.resolve();
    }

    return new Promise<void>(resolve => {
      this.waitQueue.push(resolve);
    });
  }

  release(): void {
    if (this.waitQueue.length > 0) {
      const nextResolve = this.waitQueue.shift()!;
      nextResolve();
    } else {
      this.locked = false;
    }
  }

  hasNoPendingAcquires(): boolean {
    return this.waitQueue.length === 0;
  }
}

// A map to store mutexes for each bundle name
const bundleMutexes: Map<string, Mutex> = new Map();

export interface ServerOptions {
  port?: number;
  persistencePath?: string;
  storesPath?: string;
  bundlesPath?: string;
  verbose?: boolean;
  syncInterval?: number; // Optional interval to trigger storage sync in ms
}

type DocNode = {
  type: 'doc' | 'dir';
  timestamps: {
    create: number;
    modified: number;
  };
  pointer?: DocumentId;
  children?: DocNode[];
};

export class TonkServer {
  private app: express.Application;
  private socket: WebSocketServer;
  private server: http.Server;
  private readyResolvers: ((value: any) => void)[] = [];
  private options: ServerOptions;
  // Repo is essential for the server to work but not directly referenced after initialization
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  //@ts-ignore
  private repo: Repo;
  private fsSyncTimer: NodeJS.Timeout | null = null;
  private bundleServers: Map<string, BundleServer> = new Map();
  private bundleRoutes: Map<
    string,
    {
      bundleName: string;
      bundlePath: string;
      route: string;
      id: string;
      startTime: Date;
      isRunning: boolean;
    }
  > = new Map();
  private upload: multer.Multer;
  private rootNode: RootNode;
  private bundlePersistence: BundlePersistence;

  constructor(options: ServerOptions) {
    this.options = {
      port: options.port || 7777,
      persistencePath: options.persistencePath || '',
      storesPath: options.storesPath || `${options.persistencePath}/stores`,
      bundlesPath: options.bundlesPath || `${options.persistencePath}/bundles`,
      syncInterval: options.syncInterval || 0,
      verbose: options.verbose ?? true,
    };

    // Initialize RootNode with configPath
    this.rootNode = new RootNode(
      this.options.persistencePath
        ? `${this.options.persistencePath}/root.json`
        : '',
    );

    // Initialize bundle persistence
    this.bundlePersistence = new BundlePersistence({
      persistencePath: this.options.persistencePath!,
      verbose: this.options.verbose ?? true,
    });

    this.setupDirectories();

    if (!fs.existsSync(this.options.storesPath!)) {
      fs.mkdirSync(this.options.storesPath!);
    }

    if (!fs.existsSync(this.options.bundlesPath!)) {
      fs.mkdirSync(this.options.bundlesPath!);
    }

    const hostname = os.hostname();
    this.socket = new WebSocketServer({path: '/sync', noServer: true});

    // Configure multer for file uploads
    this.upload = multer({
      storage: multer.diskStorage({
        destination: (_req, _file, cb) => {
          cb(null, os.tmpdir());
        },
        filename: (_req, _file, cb) => {
          cb(null, `${Date.now()}-${Math.round(Math.random() * 1e9)}.tar.gz`);
        },
      }),
      fileFilter: (_req, file, cb) => {
        // Accept tar.gz, tgz files and gzip MIME types
        if (
          file.mimetype === 'application/gzip' ||
          file.mimetype === 'application/x-gzip' ||
          file.mimetype === 'application/tar+gzip' ||
          file.mimetype === 'application/octet-stream' ||
          file.originalname.endsWith('.tar.gz') ||
          file.originalname.endsWith('.tgz') ||
          file.originalname.includes('tar.gz')
        ) {
          cb(null, true);
        } else {
          cb(new Error('Only .tar.gz or .tgz files are allowed'));
        }
      },
      limits: {fileSize: 500 * 1024 * 1024}, // 500MB limit
    });

    // Create Express app and HTTP server
    this.app = express();
    this.server = http.createServer(this.app);

    const config: RepoConfig = {
      network: [new NodeWSServerAdapter(this.socket as any) as any],
      storage: new NodeFSStorageAdapter(this.options.storesPath!),
      peerId: `sync-server-${hostname}` as PeerId,
      sharePolicy: async () => false,
    };

    this.repo = new Repo(config);
    this.setupExpressMiddleware();
    this.initRoot();
    this.restorePersistedBundles();
  }

  private async initRoot() {
    try {
      const createRoot = async () => {
        const docHandle = this.repo.create();
        docHandle.change((_doc: any) => {
          Object.assign(_doc, {
            type: 'dir',
            timestamps: {
              create: Date.now(),
              modified: Date.now(),
            },
            children: [],
          } as DocNode);
        });
        // Store the new document's ID
        await this.rootNode.setRootId(docHandle.documentId);
      };

      const rootId = await this.rootNode.getRootId();
      if (rootId) {
        const rootHandle = this.repo.find(rootId as DocumentId);
        const doc = await rootHandle.doc();
        if (!doc) {
          await createRoot();
        }
      } else {
        await createRoot();
      }
    } catch (err) {
      console.error('Error initializing root:', err);
      throw new Error(
        'Failed to initialize the root, this error is fatal. Shutting down server.',
      );
    }
  }

  private async restorePersistedBundles() {
    try {
      this.log('blue', 'Restoring persisted bundles...');

      // Load persisted bundle routes
      const persistedRoutes = await this.bundlePersistence.loadBundleRoutes();

      if (persistedRoutes.size === 0) {
        this.log('yellow', 'No persisted bundles found');
        return;
      }

      // Validate that the bundles still exist and filter out missing ones
      const {valid: validRoutes, removed: removedBundles} =
        await this.bundlePersistence.validateAndFilterBundles(persistedRoutes);

      if (removedBundles.length > 0) {
        this.log(
          'yellow',
          `Removed ${removedBundles.length} missing bundles: ${removedBundles.join(', ')}`,
        );
      }

      // Restore each valid bundle route
      for (const [id, bundle] of validRoutes.entries()) {
        try {
          // Check if route is already in use (shouldn't happen, but safety check)
          const existingRoute = Array.from(this.bundleRoutes.values()).find(
            r => r.route === bundle.route,
          );
          if (existingRoute) {
            this.log(
              'yellow',
              `Route ${bundle.route} is already in use, skipping bundle ${bundle.bundleName}`,
            );
            continue;
          }

          // Setup the bundle route
          this.setupBundleRoute(
            bundle.bundleName,
            bundle.bundlePath,
            bundle.route,
            bundle.id,
          );

          // Restore to the current bundle routes map
          this.bundleRoutes.set(id, bundle);

          this.log(
            'green',
            `Restored bundle ${bundle.bundleName} on route ${bundle.route}`,
          );
        } catch (error) {
          this.log(
            'red',
            `Failed to restore bundle ${bundle.bundleName}: ${error instanceof Error ? error.message : 'Unknown error'}`,
          );
        }
      }

      this.log(
        'green',
        `Successfully restored ${this.bundleRoutes.size} bundles`,
      );
    } catch (error) {
      this.log(
        'red',
        `Error restoring persisted bundles: ${error instanceof Error ? error.message : 'Unknown error'}`,
      );
      // Don't throw here - we want the server to start even if bundle restoration fails
    }
  }

  private async setupDirectories() {
    try {
      await fs.promises.access(this.options.storesPath!).catch(async () => {
        await fs.promises.mkdir(this.options.storesPath!);
      });

      await fs.promises.access(this.options.bundlesPath!).catch(async () => {
        await fs.promises.mkdir(this.options.bundlesPath!, {recursive: true});
      });
    } catch (error) {
      console.error('Error setting up directories:', error);
    }
  }

  private log(color: 'green' | 'red' | 'blue' | 'yellow', message: string) {
    if (this.options.verbose) {
      console.log(chalk[color](message));
    }
  }

  private setupExpressMiddleware() {
    // Parse JSON bodies
    this.app.use(express.json());

    // Health check endpoint
    this.app.get('/ping', (_req, res) => {
      res.send('pong');
    });

    this.app.get('/.well-known/root.json', cors(), async (_req, res) => {
      try {
        const rootFilePath = this.rootNode.getRootIdFilePath();
        // Send the file directly
        res.sendFile(rootFilePath);
      } catch (e) {
        res.status(500).json({
          error: 'Failed to retrieve root file',
          message: e instanceof Error ? e.message : 'Unknown error',
        });
      }
    });

    // Bundle upload endpoint
    this.app.post(
      '/upload-bundle',
      this.upload.single('bundle'),
      async (req, res) => {
        try {
          if (!req.file) {
            return res.status(400).send({error: 'No bundle file uploaded'});
          }

          const bundleName =
            req.body.name || path.basename(req.file.originalname, '.tar.gz');

          const bundlePath = path.join(this.options.bundlesPath!, bundleName);

          // Get or create a mutex for this bundle name
          if (!bundleMutexes.has(bundleName)) {
            bundleMutexes.set(bundleName, new Mutex());
          }
          const mutex = bundleMutexes.get(bundleName)!;

          // Acquire mutex lock to prevent race conditions
          await mutex.acquire();

          try {
            // Create bundle directory if it doesn't exist
            try {
              await fs.promises.access(bundlePath);

              // Clear existing files in the bundle directory
              const existingFiles = await fs.promises.readdir(bundlePath);
              if (existingFiles.length > 0) {
                this.log('blue', `Clearing existing files in ${bundlePath}`);
                const deletePromises = existingFiles.map(file => {
                  const filePath = path.join(bundlePath, file);
                  return fs.promises.rm(filePath, {
                    recursive: true,
                    force: true,
                  });
                });
                await Promise.all(deletePromises);
              }
            } catch {
              await fs.promises.mkdir(bundlePath, {recursive: true});
            }

            // Extract the tar.gz file
            await tar.extract({
              file: req.file.path,
              cwd: bundlePath,
              strict: true,
            });

            // Check if the bundle has a services directory
            const hasServices = await fs.promises
              .access(path.join(bundlePath, 'services'))
              .then(() => true)
              .catch(() => false);

            // Delete the temporary archive file
            await fs.promises.unlink(req.file.path);

            this.log(
              'green',
              `Bundle ${bundleName} uploaded and extracted to ${bundlePath}`,
            );

            res.status(200).send({
              success: true,
              message: 'Bundle uploaded and extracted successfully',
              bundleName,
              hasServices,
            });
          } finally {
            // Always release the mutex
            mutex.release();

            // Clean up mutex if no more uploads are pending
            if (mutex.hasNoPendingAcquires()) {
              bundleMutexes.delete(bundleName);
            }
          }
        } catch (error: any) {
          this.log('red', `Error uploading bundle: ${error.message}`);
          res.status(500).send({error: error.message});
        }
        return;
      },
    );

    // Bundle server management endpoints - now serves bundles on routes instead of separate ports
    this.app.post('/start', async (req, res) => {
      try {
        const {bundleName, route} = req.body;

        if (!bundleName) {
          return res.status(400).send({error: 'Bundle name is required'});
        }

        const bundlePath = path.join(this.options.bundlesPath!, bundleName);

        try {
          await fs.promises.access(bundlePath);
        } catch {
          return res
            .status(404)
            .send({error: `Bundle ${bundleName} not found`});
        }

        const appRoute = route || `/${bundleName}`;
        const serverId = uuidv4();

        // Check if route is already in use
        const existingRoute = Array.from(this.bundleRoutes.values()).find(
          r => r.route === appRoute,
        );
        if (existingRoute) {
          return res.status(400).send({
            error: `Route ${appRoute} is already in use by bundle ${existingRoute.bundleName}`,
          });
        }

        // Setup the bundle route on this server
        this.setupBundleRoute(bundleName, bundlePath, appRoute, serverId);

        // Store bundle route info
        this.bundleRoutes.set(serverId, {
          bundleName,
          bundlePath,
          route: appRoute,
          id: serverId,
          startTime: new Date(),
          isRunning: true,
        });

        // Persist the updated bundle routes
        this.bundlePersistence
          .saveBundleRoutes(this.bundleRoutes)
          .catch(err => {
            this.log('red', `Failed to persist bundle routes: ${err.message}`);
          });

        res.status(200).send({
          id: serverId,
          bundleName,
          route: appRoute,
          status: 'running',
          url: `http://localhost:${this.options.port}${appRoute}`,
        });
      } catch (error: any) {
        this.log('red', `Error starting bundle: ${error.message}`);
        res.status(500).send({error: error.message});
      }
      return;
    });

    this.app.post('/kill', async (req, res) => {
      try {
        const {id} = req.body;

        if (!id) {
          return res.status(400).send({error: 'Server ID is required'});
        }

        // Try to find in route-based bundles first
        const bundleRoute = this.bundleRoutes.get(id);
        if (bundleRoute) {
          // Remove the route from Express (Note: Express doesn't provide a direct way to remove routes,
          // so we mark it as stopped and it won't be served anymore)
          bundleRoute.isRunning = false;
          this.bundleRoutes.delete(id);

          // Persist the updated bundle routes
          this.bundlePersistence
            .saveBundleRoutes(this.bundleRoutes)
            .catch(err => {
              this.log(
                'red',
                `Failed to persist bundle routes after removal: ${err.message}`,
              );
            });

          this.log(
            'yellow',
            `Bundle route ${bundleRoute.route} for ${bundleRoute.bundleName} stopped`,
          );

          res.status(200).send({
            success: true,
            message: 'Bundle route stopped successfully',
          });
          return;
        }

        // Fallback to checking separate bundle servers
        const bundleServer = this.bundleServers.get(id);
        if (bundleServer) {
          await bundleServer.stop();
          this.bundleServers.delete(id);

          res
            .status(200)
            .send({success: true, message: 'Server stopped successfully'});
          return;
        }

        return res.status(404).send({error: `Bundle with ID ${id} not found`});
      } catch (error: any) {
        this.log('red', `Error stopping bundle: ${error.message}`);
        res.status(500).send({error: error.message});
      }
      return;
    });

    this.app.get('/ls', async (_req, res) => {
      try {
        const bundlesPath = this.options.bundlesPath!;

        try {
          await fs.promises.access(bundlesPath);
        } catch (err) {
          return res
            .status(404)
            .send({error: `Bundles directory ${bundlesPath} not found`});
        }

        const files = await fs.promises.readdir(bundlesPath);
        const statPromises = files.map(async file => {
          const stats = await fs.promises.stat(path.join(bundlesPath, file));
          return {file, isDirectory: stats.isDirectory()};
        });

        const fileStats = await Promise.all(statPromises);
        const bundleNames = fileStats
          .filter(item => item.isDirectory)
          .map(item => item.file);

        res.status(200).send(bundleNames);
      } catch (error: any) {
        this.log('red', `Error listing bundles: ${error.message}`);
        res.status(500).send({error: error.message});
      }
      return;
    });

    this.app.post('/delete', async (req, res) => {
      try {
        const {bundleName} = req.body;

        if (!bundleName) {
          return res.status(400).send({
            success: false,
            error: 'Bundle name is required',
          });
        }

        const bundlePath = path.join(this.options.bundlesPath!, bundleName);

        // Check if bundle exists
        if (!fs.existsSync(bundlePath)) {
          return res.status(404).send({
            success: false,
            error: `Bundle '${bundleName}' not found`,
          });
        }

        // Stop any running instances of this bundle
        const bundlesToStop: string[] = [];
        this.bundleRoutes.forEach((bundle, id) => {
          if (bundle.bundleName === bundleName) {
            bundlesToStop.push(id);
          }
        });

        for (const bundleId of bundlesToStop) {
          const bundle = this.bundleRoutes.get(bundleId);
          if (bundle) {
            bundle.isRunning = false;
            this.bundleRoutes.delete(bundleId);
            this.log(
              'yellow',
              `Stopped running instance of bundle '${bundleName}' (ID: ${bundleId})`,
            );
          }
        }

        // Delete the bundle directory from filesystem
        await fs.promises.rm(bundlePath, {recursive: true, force: true});
        this.log('green', `Deleted bundle directory: ${bundlePath}`);

        // Update persistence
        this.bundlePersistence.saveBundleRoutes(this.bundleRoutes);

        res.status(200).send({
          success: true,
          message: `Bundle '${bundleName}' deleted successfully${bundlesToStop.length > 0 ? ` (stopped ${bundlesToStop.length} running instance${bundlesToStop.length > 1 ? 's' : ''})` : ''}`,
        });

        this.log('green', `Bundle '${bundleName}' deleted successfully`);
      } catch (error: any) {
        this.log('red', `Error deleting bundle: ${error.message}`);
        res.status(500).send({
          success: false,
          error: error.message,
        });
      }
      return;
    });

    this.app.get('/ps', (_req, res) => {
      try {
        const servers: BundleServerInfo[] = [];

        // Add route-based bundles
        this.bundleRoutes.forEach((bundle, id) => {
          servers.push({
            id,
            route: bundle.route,
            bundleName: bundle.bundleName,
            status: bundle.isRunning ? 'running' : 'stopped',
            startedAt: bundle.startTime,
            url: `http://localhost:${this.options.port}${bundle.route}`,
          } as BundleServerInfo);
        });

        // Add separate bundle servers (for backward compatibility)
        this.bundleServers.forEach((server, id) => {
          const status = server.getStatus();
          servers.push({
            id,
            port: status.port,
            bundleName: status.bundleName,
            status: status.isRunning ? 'running' : 'stopped',
            startedAt: status.startTime,
          } as BundleServerInfo);
        });

        res.status(200).send(servers);
      } catch (error: any) {
        this.log('red', `Error listing processes: ${error.message}`);
        res.status(500).send({error: error.message});
      }
      return;
    });
  }

  private setupBundleRoute(
    bundleName: string,
    bundlePath: string,
    route: string,
    _serverId: string,
  ) {
    this.log('blue', `Setting up bundle route: ${route} -> ${bundlePath}`);

    // Check if there's a dist folder - if so, serve from there, otherwise serve from the bundle root
    const distPath = path.join(bundlePath, 'dist');
    const servePath = fs.existsSync(distPath) ? distPath : bundlePath;

    this.log('blue', `Serving static files from: ${servePath}`);

    // Setup API proxies for this bundle
    this.setupBundleApiProxies(bundlePath, route);

    // Handle WASM files with correct MIME type for this route
    this.app.get(`${route}/*.wasm`, (_req, res, next) => {
      res.set('Content-Type', 'application/wasm');
      next();
    });

    // Serve static files from the appropriate directory (dist if available, otherwise bundle root)
    this.app.use(route, express.static(servePath));

    // Client-side routing - for SPA, send index.html for all non-api paths under this route
    this.app.get(
      new RegExp(
        `^${route.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}(?!\\/api|\\/services)(?!\\.\\w+$).*$`,
      ),
      (req, res) => {
        if (req.path === `${route}/.well-known/root.json`) {
          return res.sendFile(this.rootNode.getRootIdFilePath());
        }
        res.sendFile(path.join(servePath, 'index.html'));
      },
    );

    this.log(
      'green',
      `Bundle route setup complete: ${route} serving ${bundleName}`,
    );
  }

  private setupBundleApiProxies(bundlePath: string, route: string) {
    const apiServicesPath = path.join(bundlePath, 'apiServices.json');

    // Check if apiServices.json exists
    if (!fs.existsSync(apiServicesPath)) {
      return;
    }

    try {
      // Read and parse the apiServices.json file
      const apiServicesContent = fs.readFileSync(apiServicesPath, 'utf8');
      const apiServices = JSON.parse(apiServicesContent);

      if (!Array.isArray(apiServices) || apiServices.length === 0) {
        return;
      }

      // Set up a proxy for each API service under the bundle route
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
          continue;
        }

        const proxyPath = `${route}/api/${prefix}`;
        this.log(
          'blue',
          `Setting up API proxy for bundle: ${proxyPath} -> ${baseUrl}`,
        );

        this.app.use(
          proxyPath,
          createProxyMiddleware({
            target: baseUrl,
            changeOrigin: true,
            pathRewrite: {
              [`^${route}/api/${prefix}`]: '', // Remove the route prefix when forwarding
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
                  } else if (authType === 'query' && authQueryParamName) {
                    // Handle query parameter authentication
                    const url = new URL(req.url!, 'http://localhost');
                    url.searchParams.set(authQueryParamName, authValue || '');
                    req.url = url.pathname + url.search;
                  }
                }

                if (this.options.verbose) {
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
    } catch (error) {
      this.log('red', `Error setting up API proxies for ${route}: ${error}`);
    }
  }

  public start(): Promise<void> {
    return new Promise(async resolve => {
      this.server.listen(this.options.port, () => {
        this.log('green', `Server running on port ${this.options.port}`);

        this.readyResolvers.forEach((resolve: any) => resolve(true));

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

    // Stop all bundle servers
    const stopPromises: Promise<void>[] = [];
    this.bundleServers.forEach(server => {
      stopPromises.push(server.stop());
    });

    await Promise.all(stopPromises);
    this.bundleServers.clear();

    // Clear route-based bundles
    this.bundleRoutes.clear();

    return new Promise((resolve, reject) => {
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
