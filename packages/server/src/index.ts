import express from 'express';
import http from 'node:http';
import path from 'node:path';
import chalk from 'chalk';
import fs from 'node:fs';
import os from 'node:os';
import {WebSocketServer} from 'ws';
import {NodeWSServerAdapter} from '@automerge/automerge-repo-network-websocket';
import {NodeFSStorageAdapter} from '@automerge/automerge-repo-storage-nodefs';
import multer from 'multer';
import * as tar from 'tar';
import cors from 'cors';
import {v4 as uuidv4} from 'uuid';
import {createProxyMiddleware} from 'http-proxy-middleware';
import {RootNode} from './rootNode.js';
import {BundlePersistence} from './bundlePersistence.js';
import {NginxManager} from './nginxManager.js';
import {PortAllocator} from './portAllocator.js';
import {ServerManager} from './serverManager.js';
import {Repo} from '@automerge/automerge-repo';

import type {PeerId, RepoConfig, DocumentId} from '@automerge/automerge-repo';
import type {BundleServerInfo} from './types.js';

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

  private bundleRoutes: Map<
    string,
    {
      bundleName: string;
      bundlePath: string;
      route: string;
      id: string;
      startTime: Date;
      isRunning: boolean;
      serverPort?: number;
    }
  > = new Map();
  private upload: multer.Multer;
  private rootNode: RootNode;
  private bundlePersistence: BundlePersistence;
  private nginxManager?: NginxManager;
  private portAllocator: PortAllocator;
  private serverManager: ServerManager;

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

    // Initialize port allocator
    this.portAllocator = new PortAllocator();

    // Initialize server manager
    this.serverManager = new ServerManager();

    // Initialize nginx manager (skip in development)
    if (process.env.NODE_ENV !== 'development') {
      this.nginxManager = new NginxManager();
    }

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
          const serverPort = await this.setupBundleRoute(
            bundle.bundleName,
            bundle.bundlePath,
            bundle.route,
            bundle.id,
          );

          // Create updated bundle info with server port
          const updatedBundle: {
            bundleName: string;
            bundlePath: string;
            route: string;
            id: string;
            startTime: Date;
            isRunning: boolean;
            serverPort?: number;
          } = {
            ...bundle,
          };

          if (serverPort !== undefined) {
            updatedBundle.serverPort = serverPort;
          }

          // Restore to the current bundle routes map
          this.bundleRoutes.set(id, updatedBundle);

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

    // Note: Global nginx proxy setup moved to start() method after nginx starts

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

            // Delete the temporary archive file
            await fs.promises.unlink(req.file.path);

            // Check if server folder exists and run npm install
            const serverPath = path.join(bundlePath, 'server');
            try {
              await fs.promises.access(serverPath);
              this.log(
                'blue',
                `Server folder found for bundle ${bundleName}, running npm install...`,
              );

              // Run npm install in the server directory
              const {spawn} = await import('node:child_process');
              const npmInstall = spawn('npm', ['install'], {
                cwd: serverPath,
                stdio: 'pipe',
              });

              let output = '';
              let errorOutput = '';

              npmInstall.stdout?.on('data', data => {
                output += data.toString();
              });

              npmInstall.stderr?.on('data', data => {
                errorOutput += data.toString();
              });

              await new Promise<void>((resolve, reject) => {
                npmInstall.on('close', code => {
                  if (code === 0) {
                    this.log(
                      'green',
                      `npm install completed successfully for bundle ${bundleName}`,
                    );
                    resolve();
                  } else {
                    this.log(
                      'red',
                      `npm install failed for bundle ${bundleName} with code ${code}`,
                    );
                    this.log('red', `Error output: ${errorOutput}`);
                    reject(
                      new Error(
                        `npm install failed with code ${code}: ${errorOutput}`,
                      ),
                    );
                  }
                });

                npmInstall.on('error', err => {
                  this.log(
                    'red',
                    `Failed to start npm install for bundle ${bundleName}: ${err.message}`,
                  );
                  reject(err);
                });
              });
            } catch (serverAccessError) {
              // Server folder doesn't exist, skip npm install
              this.log(
                'yellow',
                `No server folder found for bundle ${bundleName}, skipping npm install`,
              );
            }

            this.log(
              'green',
              `Bundle ${bundleName} uploaded and extracted to ${bundlePath}`,
            );

            res.status(200).send({
              success: true,
              message: 'Bundle uploaded and extracted successfully',
              bundleName,
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
        const serverPort = await this.setupBundleRoute(
          bundleName,
          bundlePath,
          appRoute,
          serverId,
        );

        // Store bundle route info
        const bundleInfo: {
          bundleName: string;
          bundlePath: string;
          route: string;
          id: string;
          startTime: Date;
          isRunning: boolean;
          serverPort?: number;
        } = {
          bundleName,
          bundlePath,
          route: appRoute,
          id: serverId,
          startTime: new Date(),
          isRunning: true,
        };

        if (serverPort !== undefined) {
          bundleInfo.serverPort = serverPort;
        }

        this.bundleRoutes.set(serverId, bundleInfo);

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
          // Stop the bundle server if it's running
          await this.stopBundleServer(
            bundleRoute.bundleName,
            bundleRoute.serverPort,
          );

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
            // Stop the bundle server
            await this.stopBundleServer(bundle.bundleName, bundle.serverPort);

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

        res.status(200).send(servers);
      } catch (error: any) {
        this.log('red', `Error listing processes: ${error.message}`);
        res.status(500).send({error: error.message});
      }
      return;
    });
  }

  private async setupGlobalNginxProxy() {
    // Set up single proxy for all /api requests to nginx
    this.log('blue', 'Setting up global nginx proxy for /api requests');

    if (!this.nginxManager?.getStatus().isRunning) {
      return;
    }

    // Global proxy for all /api requests
    this.app.use(
      '/api',
      createProxyMiddleware({
        target: 'http://localhost:8080', // Dedicated nginx server
        changeOrigin: true,
        on: {
          error: (err, _req, res) => {
            this.log('red', `Nginx proxy error: ${err.message}`);
            if (res && 'writeHead' in res && 'headersSent' in res) {
              if (!res.headersSent) {
                res.writeHead(500);
                res.end(
                  JSON.stringify({
                    error: 'Nginx proxy error',
                    message: err.message,
                  }),
                );
              }
            }
          },
        },
      }),
    );
  }

  private async startBundleServer(
    bundleName: string,
    bundlePath: string,
  ): Promise<number | undefined> {
    const serverPath = path.join(bundlePath, 'server');

    const serverRoutesPath = path.join(serverPath, 'server-routes.json');
    // Check if server directory exists
    if (!fs.existsSync(serverRoutesPath)) {
      this.log('yellow', `No server directory found for bundle ${bundleName}`);
      return undefined;
    }

    try {
      const content = await fs.promises.readFile(serverRoutesPath);
      const routes = JSON.parse(content.toString());
      //if you just have ping, then we ignore
      //if you've replaced with something random, we also ignore
      if (routes.length === 1) {
        this.log(
          'yellow',
          `No server routes to proxy detected for this bundle ${bundleName}.`,
        );
        return;
      }
    } catch (e) {
      this.log('red', `Error reading ${serverRoutesPath} with error ${e}`);
      return undefined;
    }

    this.log('blue', `Starting server for bundle ${bundleName}`);

    // Allocate a port for the bundle server
    const serverPort = await this.portAllocator.allocate();
    this.log('blue', `Allocated port ${serverPort} for bundle ${bundleName}`);

    // Start the server process
    try {
      await this.serverManager.startServer({
        bundleName,
        serverPath,
        port: serverPort,
      });
      this.log(
        'green',
        `Bundle server started for ${bundleName} on port ${serverPort}`,
      );
    } catch (error) {
      this.log(
        'red',
        `Failed to start bundle server for ${bundleName}: ${error}`,
      );
      // Deallocate the port if server startup fails
      await this.portAllocator.deallocate(serverPort);
      return undefined;
    }

    // Check for nginx config and deploy it
    const configPath = path.join(serverPath, `app-${bundleName}.conf`);
    if (fs.existsSync(configPath)) {
      this.log('blue', `Loading nginx config for bundle ${bundleName}`);

      // Read the nginx config file and replace ${port} with allocated port
      try {
        let configContent = await fs.promises.readFile(configPath, 'utf8');
        configContent = configContent
          .replace(/\$\{port\}/g, serverPort.toString())
          .replace(/\$\{bundleName\}/g, bundleName)
          .replace(/\$\{bundlePath\}/g, bundlePath);

        this.log(
          'blue',
          `Processed nginx config for bundle ${bundleName}, replaced placeholders with port ${serverPort}`,
        );

        if (this.nginxManager) {
          try {
            await this.nginxManager.deployAppConfig(bundleName, configContent);
            this.log(
              'green',
              `Nginx config deployed for bundle ${bundleName} on port ${serverPort}`,
            );
          } catch (error) {
            this.log(
              'red',
              `Failed to deploy nginx config for ${bundleName}: ${error}`,
            );
            // Deallocate the port if nginx config fails
            await this.portAllocator.deallocate(serverPort);
            return undefined;
          }
        }
      } catch (error) {
        this.log(
          'red',
          `Failed to read nginx config for ${bundleName}: ${error}`,
        );
        // Deallocate the port if config reading fails
        await this.portAllocator.deallocate(serverPort);
        return undefined;
      }
    }

    return serverPort;
  }

  private async stopBundleServer(
    bundleName: string,
    serverPort?: number,
  ): Promise<void> {
    this.log('blue', `Stopping server for bundle ${bundleName}`);

    // Stop the server process if it's running
    try {
      await this.serverManager.stopServer(bundleName);
      this.log('green', `Bundle server stopped for ${bundleName}`);
    } catch (error) {
      this.log(
        'red',
        `Failed to stop bundle server for ${bundleName}: ${error}`,
      );
    }

    // Remove nginx config
    if (this.nginxManager) {
      try {
        await this.nginxManager.removeAppConfig(bundleName);
        this.log('green', `Nginx config removed for bundle ${bundleName}`);
      } catch (error) {
        this.log(
          'red',
          `Failed to remove nginx config for ${bundleName}: ${error}`,
        );
      }
    }

    // Deallocate the port if it was allocated
    if (serverPort !== undefined) {
      await this.portAllocator.deallocate(serverPort);
      this.log(
        'blue',
        `Deallocated port ${serverPort} for bundle ${bundleName}`,
      );
    }
  }

  private async setupBundleRoute(
    bundleName: string,
    bundlePath: string,
    route: string,
    _serverId: string,
  ): Promise<number | undefined> {
    this.log('blue', `Setting up bundle route: ${route} -> ${bundlePath}`);

    // Check if there's a dist folder - if so, serve from there, otherwise serve from the bundle root
    const distPath = path.join(bundlePath, 'dist');

    this.log('blue', `Serving static files from: ${distPath}`);

    // Handle WASM files with correct MIME type for this route
    this.app.get(`${route}/*.wasm`, (_req, res, next) => {
      res.set('Content-Type', 'application/wasm');
      next();
    });

    // Start bundle server if it exists and get the allocated port
    const serverPort = await this.startBundleServer(bundleName, bundlePath);

    // Store the server port for later use
    if (serverPort) {
      this.log(
        'blue',
        `Bundle ${bundleName} server started on port ${serverPort}`,
      );
    }

    // Serve static files from the appropriate directory (dist if available, otherwise bundle root)
    this.app.use(route, express.static(distPath));

    // Client-side routing - for SPA, send index.html for all non-api paths under this route
    this.app.get(
      new RegExp(
        `^${route.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}(?!\\/api|\\/services)(?!\\.\\w+$).*$`,
      ),
      (req, res) => {
        if (req.path === `${route}/.well-known/root.json`) {
          return res.sendFile(this.rootNode.getRootIdFilePath());
        }
        res.sendFile(path.join(distPath, 'index.html'));
      },
    );

    this.log(
      'green',
      `Bundle route setup complete: ${route} serving ${bundleName}`,
    );

    return serverPort;
  }

  public start(): Promise<void> {
    return new Promise(async resolve => {
      try {
        // Start nginx server first (skip in development)
        if (this.nginxManager) {
          await this.nginxManager.start();

          // Set up global nginx proxy for all /api requests after nginx starts
          await this.setupGlobalNginxProxy();
        }

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
      } catch (error) {
        this.log('red', `Failed to start nginx server: ${error}`);
        throw error;
      }
    });
  }

  public async stop(): Promise<void> {
    if (this.fsSyncTimer) {
      clearInterval(this.fsSyncTimer);
      this.fsSyncTimer = null;
    }

    // Stop all running bundle servers
    await this.serverManager.stopAllServers();

    // Clear route-based bundles
    this.bundleRoutes.clear();

    // Stop nginx server (skip in development)
    if (this.nginxManager) {
      await this.nginxManager.stop();
    }

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
