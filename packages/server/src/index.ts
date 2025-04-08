import {WebSocketServer, WebSocket} from 'ws';
import express from 'express';
import http from 'http';
import path from 'path';
import chalk from 'chalk';
import fs from 'fs';
import {BackblazeStorageMiddleware} from './backblazeMiddleware.js';
import {BackblazeStorageMiddlewareOptions} from './backblazeStorage.js';
import {FileSystemStorageMiddleware} from './filesystemMiddleware.js';
import {FileSystemStorageOptions} from './filesystemStorage.js';
import {createProxyMiddleware} from 'http-proxy-middleware';
import cors from 'cors';
import {loadIntegrations} from './workerManager.js';

export interface ServerOptions {
  port?: number;
  mode: 'development' | 'production';
  distPath: string | undefined; // Path to the built frontend files
  verbose?: boolean;
  storage?: BackblazeStorageMiddlewareOptions | undefined;
  filesystemStorage?: FileSystemStorageOptions | undefined;
  syncInterval?: number; // Optional interval to trigger storage sync in ms
  primaryStorage?: 'backblaze' | 'filesystem'; // Which storage to use as primary (default: backblaze if both configured)
  apiProxy?:
    | {
        target: string; // The target URL to proxy to (e.g., 'http://localhost:3001')
        ws?: boolean; // Whether to proxy WebSocket connections
      }
    | undefined;
}

export class TonkServer {
  private app: express.Application;
  private server: http.Server;
  private wss: WebSocketServer;
  private connections: Map<WebSocket, Set<string>> = new Map(); // Map of connections to subscribed document IDs
  private options: ServerOptions;
  private storageMiddleware: BackblazeStorageMiddleware | null = null;
  private filesystemMiddleware: FileSystemStorageMiddleware | null = null;
  private backblazeSyncTimer: NodeJS.Timeout | null = null;
  private fsSyncTimer: NodeJS.Timeout | null = null;

  constructor(options: ServerOptions) {
    this.options = {
      port: options.port || (options.mode === 'development' ? 4080 : 8080),
      mode: options.mode,
      distPath: options.distPath,
      verbose: options.verbose ?? true,
      storage: options.storage,
      filesystemStorage: options.filesystemStorage,
      syncInterval: options.syncInterval || 5 * 60 * 1000, // Default: 5 minutes
      primaryStorage:
        options.primaryStorage ||
        (options.storage ? 'backblaze' : 'filesystem'),
      apiProxy: options.apiProxy,
    };

    // Create Express app and HTTP server
    this.app = express();
    this.server = http.createServer(this.app);

    // Create WebSocket server
    this.wss = new WebSocketServer({
      server: this.server,
      path: '/sync',
    });

    // Initialize Backblaze storage middleware if configured
    if (this.options.storage) {
      this.storageMiddleware = new BackblazeStorageMiddleware(
        this.options.storage,
        'TonkServer-Backblaze',
        this.options.verbose,
      );
    }

    // Initialize Filesystem storage middleware if configured
    if (this.options.filesystemStorage) {
      this.filesystemMiddleware = new FileSystemStorageMiddleware(
        this.options.filesystemStorage,
        'TonkServer-Filesystem',
        this.options.verbose,
      );
    }

    this.setupWebSocketHandlers();
    this.setupExpressMiddleware();
    this.setupStorageSyncTimers();
  }

  private log(color: 'green' | 'red' | 'blue' | 'yellow', message: string) {
    if (this.options.verbose) {
      console.log(chalk[color](message));
    }
  }

  private setupStorageSyncTimers(): void {
    // Set up Backblaze sync timer if configured
    if (this.options.syncInterval && this.storageMiddleware) {
      this.backblazeSyncTimer = setInterval(() => {
        this.storageMiddleware!.forceSyncToBackblaze().catch(error => {
          this.log('red', `Scheduled Backblaze sync failed: ${error.message}`);
        });
      }, this.options.syncInterval);
    }

    // Set up Filesystem sync timer if configured
    if (this.options.syncInterval && this.filesystemMiddleware) {
      this.fsSyncTimer = setInterval(() => {
        this.filesystemMiddleware!.forceSyncToFileSystem().catch(error => {
          this.log('red', `Scheduled Filesystem sync failed: ${error.message}`);
        });
      }, this.options.syncInterval);
    }
  }

  private setupWebSocketHandlers() {
    // Handle WebSocket connections
    this.wss.on('connection', (ws: WebSocket) => {
      this.log('green', `Client connected`);
      this.connections.set(ws, new Set());

      // Determine which storage middleware to use for initial document loading
      const primaryMiddleware = this.getPrimaryStorageMiddleware();

      // Send stored documents to the new client if a primary storage middleware is enabled
      if (primaryMiddleware && primaryMiddleware.isInitialized()) {
        // Get all document IDs
        const docIds = primaryMiddleware.getAllDocumentIds();

        if (docIds.length > 0) {
          this.log(
            'blue',
            `Sending ${docIds.length} stored documents to new client`,
          );

          // Send each document to the client
          for (const docId of docIds) {
            this.sendDocumentToClient(ws, docId);
          }
        }
      }

      // Handle messages from clients
      ws.on('message', async (data: Buffer) => {
        try {
          let messageData: any;
          try {
            // Try to parse the message
            messageData = JSON.parse(data.toString());
          } catch (error) {
            // Not JSON, treat as binary data
            messageData = null;
          }

          // First, forward the raw message immediately to all other clients for real-time sync
          this.connections.forEach((_, client) => {
            if (client !== ws && client.readyState === WebSocket.OPEN) {
              client.send(data);
            }
          });

          // Don't continue with storage if it's not a JSON message or doesn't have docId
          if (!messageData || !messageData.docId || !messageData.changes) {
            return;
          }

          // Process through primary storage middleware first if available
          const primaryMiddleware = this.getPrimaryStorageMiddleware();
          if (primaryMiddleware) {
            const result = await primaryMiddleware.handleIncomingChanges(
              messageData.docId,
              messageData.changes,
            );

            if (result && result.didChange) {
              // Track that this client is subscribed to this document
              const subscriptions = this.connections.get(ws);
              if (subscriptions) {
                subscriptions.add(messageData.docId);
              }

              // Also propagate to secondary storage if available
              await this.propagateToSecondaryStorage(
                messageData.docId,
                messageData.changes,
              );
            }
          }
        } catch (error) {
          this.log(
            'red',
            `Error processing message: ${(error as Error).message}`,
          );
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

  // Get the primary storage middleware based on configuration
  private getPrimaryStorageMiddleware():
    | BackblazeStorageMiddleware
    | FileSystemStorageMiddleware
    | null {
    if (this.options.primaryStorage === 'backblaze' && this.storageMiddleware) {
      return this.storageMiddleware;
    } else if (
      this.options.primaryStorage === 'filesystem' &&
      this.filesystemMiddleware
    ) {
      return this.filesystemMiddleware;
    } else if (this.storageMiddleware) {
      return this.storageMiddleware;
    } else if (this.filesystemMiddleware) {
      return this.filesystemMiddleware;
    }
    return null;
  }

  // Get the secondary storage middleware
  private getSecondaryStorageMiddleware():
    | BackblazeStorageMiddleware
    | FileSystemStorageMiddleware
    | null {
    if (
      this.options.primaryStorage === 'backblaze' &&
      this.filesystemMiddleware
    ) {
      return this.filesystemMiddleware;
    } else if (
      this.options.primaryStorage === 'filesystem' &&
      this.storageMiddleware
    ) {
      return this.storageMiddleware;
    }
    return null;
  }

  // Propagate changes to secondary storage if available
  private async propagateToSecondaryStorage(
    docId: string,
    changes: Uint8Array | number[],
  ): Promise<void> {
    const secondaryMiddleware = this.getSecondaryStorageMiddleware();
    if (secondaryMiddleware) {
      try {
        await secondaryMiddleware.handleIncomingChanges(docId, changes);
      } catch (error) {
        this.log(
          'red',
          `Error propagating changes to secondary storage: ${
            (error as Error).message
          }`,
        );
      }
    }
  }

  private async sendDocumentToClient(
    ws: WebSocket,
    docId: string,
  ): Promise<void> {
    const primaryMiddleware = this.getPrimaryStorageMiddleware();
    if (!primaryMiddleware) return;

    try {
      // Track that this client is subscribed to this document
      const subscriptions = this.connections.get(ws);
      if (subscriptions) {
        subscriptions.add(docId);
      }

      // Generate a sync message for this client
      // This will use in-memory documents when available
      const syncMessage = await primaryMiddleware.generateSyncMessage(docId);

      if (syncMessage && syncMessage.length > 0) {
        const message = {
          docId,
          changes: Array.from(syncMessage),
        };

        this.log(
          'blue',
          `Sending document ${docId} sync message to client (${syncMessage.length} bytes)`,
        );
        ws.send(JSON.stringify(message));
      } else {
        this.log(
          'yellow',
          `No sync message generated for document ${docId}, document may be empty`,
        );
      }
    } catch (error) {
      this.log(
        'red',
        `Error sending document ${docId}: ${(error as Error).message}`,
      );
    }
  }

  private async setupExpressMiddleware() {
    // If apiProxy is configured, use proxy middleware instead of local router
    this.log('red', JSON.stringify(this.options.apiProxy));
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

    this.app.get('/ping', (_req, res) => {
      res.send('pong');
    });

    // In production mode, serve static files and handle client-side routing
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

  public start(): Promise<void> {
    return new Promise(async resolve => {
      if (this.options.mode === 'production' && this.options.distPath) {
        await this.updateWasmCache();
      }

      this.server.listen(this.options.port, () => {
        this.log(
          'green',
          `Server running on port ${this.options.port}, update: 5`,
        );

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
      // Stop the sync timers
      if (this.backblazeSyncTimer) {
        clearInterval(this.backblazeSyncTimer);
        this.backblazeSyncTimer = null;
      }

      if (this.fsSyncTimer) {
        clearInterval(this.fsSyncTimer);
        this.fsSyncTimer = null;
      }

      // Shut down storage middlewares if enabled
      if (this.storageMiddleware) {
        try {
          await this.storageMiddleware.shutdown();
        } catch (error) {
          this.log(
            'red',
            `Error shutting down Backblaze middleware: ${
              (error as Error).message
            }`,
          );
        }
      }

      if (this.filesystemMiddleware) {
        try {
          await this.filesystemMiddleware.shutdown();
        } catch (error) {
          this.log(
            'red',
            `Error shutting down Filesystem middleware: ${
              (error as Error).message
            }`,
          );
        }
      }

      this.connections.forEach((_, conn) => {
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

  // Force sync all documents to Backblaze
  public async forceSyncToBackblaze(): Promise<void> {
    if (this.storageMiddleware) {
      return this.storageMiddleware.forceSyncToBackblaze();
    }
  }

  // Reload all documents from Backblaze
  public async reloadFromBackblaze(): Promise<void> {
    if (this.storageMiddleware) {
      return this.storageMiddleware.reloadFromBackblaze();
    }
  }

  // Force sync all documents to Filesystem
  public async forceSyncToFileSystem(): Promise<void> {
    if (this.filesystemMiddleware) {
      return this.filesystemMiddleware.forceSyncToFileSystem();
    }
  }

  // Reload all documents from Filesystem
  public async reloadFromFileSystem(): Promise<void> {
    if (this.filesystemMiddleware) {
      return this.filesystemMiddleware.reloadFromFileSystem();
    }
  }

  // Sync document from primary to secondary storage
  public async syncDocumentBetweenStorages(docId: string): Promise<boolean> {
    const primary = this.getPrimaryStorageMiddleware();
    const secondary = this.getSecondaryStorageMiddleware();

    if (!primary || !secondary) {
      this.log(
        'yellow',
        'Cannot sync between storages - both storages need to be configured',
      );
      return false;
    }

    try {
      // Get document from primary storage
      const doc = await primary.getDocument(docId);
      if (!doc) {
        this.log('yellow', `Document ${docId} not found in primary storage`);
        return false;
      }

      // Get sync message from primary
      const syncMessage = await primary.generateSyncMessage(docId);
      if (!syncMessage) {
        this.log('yellow', `No sync message generated for document ${docId}`);
        return false;
      }

      // Apply to secondary
      const result = await secondary.handleIncomingChanges(docId, syncMessage);

      this.log(
        'green',
        `Document ${docId} synced between storages (changes: ${result.didChange})`,
      );
      return result.didChange;
    } catch (error) {
      this.log(
        'red',
        `Error syncing document ${docId} between storages: ${
          (error as Error).message
        }`,
      );
      return false;
    }
  }

  // Sync all documents between primary and secondary storage
  public async syncAllDocumentsBetweenStorages(): Promise<number> {
    const primary = this.getPrimaryStorageMiddleware();
    const secondary = this.getSecondaryStorageMiddleware();

    if (!primary || !secondary) {
      this.log(
        'yellow',
        'Cannot sync between storages - both storages need to be configured',
      );
      return 0;
    }

    const docIds = primary.getAllDocumentIds();
    let syncedCount = 0;

    for (const docId of docIds) {
      const success = await this.syncDocumentBetweenStorages(docId);
      if (success) syncedCount++;
    }

    this.log(
      'green',
      `Synced ${syncedCount} of ${docIds.length} documents between storages`,
    );
    return syncedCount;
  }
}

// Convenience function to create and start a server
export async function createServer(
  options: ServerOptions,
): Promise<TonkServer> {
  const server = new TonkServer(options);
  await server.start();
  loadIntegrations();
  return server;
}
