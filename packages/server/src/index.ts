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
import {BundleServer} from './bundleServer.js';
import {BundleServerInfo} from './types.js';
import {RootNode} from './rootNode.js';
// use env-path to store the rootId file

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
  dirPath?: string;
  bundlesPath?: string; // Path to store bundle files
  verbose?: boolean;
  syncInterval?: number; // Optional interval to trigger storage sync in ms
  configPath?: string;
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
  private upload: multer.Multer;
  private rootNode: RootNode;

  constructor(options: ServerOptions) {
    this.options = {
      port: options.port || 7777,
      dirPath: options.dirPath || 'stores',
      bundlesPath: options.bundlesPath || 'bundles',
      syncInterval: options.syncInterval || 0,
      verbose: options.verbose ?? true,
      configPath: options.configPath || '',
    };

    // Initialize RootNode with configPath
    this.rootNode = new RootNode(this.options.configPath);

    this.setupDirectories();

    if (!fs.existsSync(this.options.dirPath!)) {
      fs.mkdirSync(this.options.dirPath!);
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
        // Accept only tar.gz and tgz files
        if (
          file.mimetype === 'application/gzip' ||
          file.mimetype === 'application/x-gzip' ||
          file.mimetype === 'application/tar+gzip' ||
          file.originalname.endsWith('.tar.gz') ||
          file.originalname.endsWith('.tgz')
        ) {
          cb(null, true);
        } else {
          cb(null, false);
          cb(new Error('Only .tar.gz or .tgz files are allowed'));
        }
      },
      limits: {fileSize: 500 * 1024 * 1024}, // 500MB limit
    });

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
    this.initRoot();
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

  private async setupDirectories() {
    try {
      await fs.promises.access(this.options.dirPath!).catch(async () => {
        await fs.promises.mkdir(this.options.dirPath!);
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

    // Bundle server management endpoints
    this.app.post('/start', async (req, res) => {
      try {
        const {bundleName, port} = req.body;

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

        const hasServices = await fs.promises
          .access(path.join(bundlePath, 'services'))
          .then(() => true)
          .catch(() => false);

        const serverPort = port || this.findAvailablePort();
        const serverId = uuidv4();

        const bundleServer = new BundleServer({
          bundleName,
          bundlePath,
          port: serverPort,
          hasServices,
          rootNode: this.rootNode,
          verbose:
            this.options.verbose === undefined ? true : this.options.verbose,
        });

        await bundleServer.start();
        this.bundleServers.set(serverId, bundleServer);

        res.status(200).send({
          id: serverId,
          bundleName,
          port: serverPort,
          status: 'running',
        });
      } catch (error: any) {
        this.log('red', `Error starting bundle server: ${error.message}`);
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

        const bundleServer = this.bundleServers.get(id);

        if (!bundleServer) {
          return res
            .status(404)
            .send({error: `Server with ID ${id} not found`});
        }

        await bundleServer.stop();
        this.bundleServers.delete(id);

        res
          .status(200)
          .send({success: true, message: 'Server stopped successfully'});
      } catch (error: any) {
        this.log('red', `Error stopping bundle server: ${error.message}`);
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

    this.app.get('/ps', (_req, res) => {
      const servers: BundleServerInfo[] = [];

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
    });
  }

  // Find an available port starting from 8000
  private findAvailablePort(startPort: number = 8000): number {
    for (let port = startPort; port < startPort + 1000; port++) {
      const inUse = Array.from(this.bundleServers.values()).some(
        server =>
          server.getStatus().port === port && server.getStatus().isRunning,
      );

      if (!inUse) {
        return port;
      }
    }

    // If we can't find an available port, return a random one
    return startPort + Math.floor(Math.random() * 1000);
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
