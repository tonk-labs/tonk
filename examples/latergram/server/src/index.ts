import {
  AnyDocumentId,
  DocHandle,
  Repo,
  type RepoConfig,
} from '@automerge/automerge-repo';
import { NodeWSServerAdapter } from '@automerge/automerge-repo-network-websocket';
import cors from 'cors';
import express from 'express';
import { readFileSync } from 'fs';
import { dirname, join } from 'path';
import { fileURLToPath } from 'url';
import { WebSocketServer } from 'ws';
import { BundleStorageAdapter } from './bundleStorageAdapter.js';

class Server {
  #socket: WebSocketServer;

  #server: ReturnType<import('express').Express['listen']>;
  #storage: BundleStorageAdapter;

  #repo: Repo;
  #rootDocumentId: string | null = null;

  static async create(port: number, bundlePath: string): Promise<Server> {
    // Read bundle file as bytes
    const bundleBytes = readFileSync(bundlePath);

    // Create storage adapter from bundle
    const storage = await BundleStorageAdapter.fromBundle(bundleBytes);

    return new Server(port, storage);
  }

  private constructor(port: number, storage: BundleStorageAdapter) {
    this.#socket = new WebSocketServer({ noServer: true });

    const PORT = port;
    const app = express();

    // Enable CORS for all routes to allow browser clients to fetch root document

    app.use(
      cors({
        origin: true, // Allow all origins
        credentials: true, // Allow cookies/credentials
        methods: ['GET', 'POST', 'PUT', 'DELETE', 'OPTIONS'],
        allowedHeaders: ['Content-Type', 'Authorization'],
      })
    );

    app.use(express.static('public'));
    this.#storage = storage;

    const config: RepoConfig = {
      network: [new NodeWSServerAdapter(this.#socket as any) as any],
      storage: this.#storage,
      /** @ts-expect-error @type {(import("automerge-repo").PeerId)}  */
      peerId: `storage-server` as PeerId,
      // Since this is a server, we don't share generously — meaning we only sync documents they already
      // know about and can ask for by ID.
      sharePolicy: async () => true,
    };
    const serverRepo = new Repo(config);
    this.#repo = serverRepo;

    app.get('/', (_req, res) => {
      res.send(`👍 @automerge/automerge-repo-sync-server is running`);
    });

    app.get('/tonk_core_bg.wasm', (_req, res) => {
      try {
        // Read the WASM file from the server directory
        const __filename = fileURLToPath(import.meta.url);
        const __dirname = dirname(__filename);
        const wasmPath = join(
          __dirname,
          '..',
          '..',
          '..',
          '..',
          'packages',
          'core-js',
          'dist',
          'tonk_core_bg.wasm'
        );
        const wasmBuffer = readFileSync(wasmPath);

        // Set headers similar to esm.sh
        res.setHeader('Content-Type', 'application/wasm');
        res.setHeader('Cache-Control', 'public, max-age=31536000, immutable');
        res.setHeader('Access-Control-Allow-Origin', '*');
        res.setHeader('Access-Control-Allow-Methods', 'GET, HEAD, OPTIONS');
        res.setHeader('Content-Length', wasmBuffer.length.toString());

        // Send the WASM file
        res.send(wasmBuffer);
      } catch (error) {
        console.error('Error serving WASM file:', error);
        res.status(404).json({ error: 'WASM file not found' });
      }
    });

    // Endpoint to get the manifest as a slim bundle (zip file with just manifest.json)
    app.get('/.manifest.tonk', async (_req, res) => {
      console.log('Received request for /.manifest.tonk');
      try {
        console.log('Creating slim bundle...');
        const slimBundle = await this.#storage.createSlimBundle();
        console.log(
          'Slim bundle created successfully, size:',
          slimBundle.length
        );

        // Set appropriate headers for zip file download
        res.setHeader('Content-Type', 'application/zip');
        res.setHeader(
          'Content-Disposition',
          'attachment; filename="manifest.tonk"'
        );
        res.setHeader('Content-Length', slimBundle.length.toString());

        res.send(Buffer.from(slimBundle));
        console.log('Slim bundle sent successfully');
      } catch (error) {
        console.error('Error creating slim bundle:', error);
        res.status(500).json({ error: 'Failed to create manifest bundle' });
      }
    });

    this.#server = app.listen(PORT, () => {
      const address = this.#server.address();
      console.log(
        `Listening on port ${typeof address === 'string' ? address : address?.port}`
      );
    });

    this.#server.on('upgrade', (request, socket, head) => {
      console.log('upgrading to websocket');
      this.#socket.handleUpgrade(request, socket, head, socket => {
        this.#socket.emit('connection', socket, request);
      });
    });
  }

  close() {
    this.#storage.log();
    this.#socket.close();
    this.#server.close();
  }
}

async function main() {
  const port = process.argv[2] ? parseInt(process.argv[2]) : 8080;
  const bundlePath = process.argv[3];

  if (!bundlePath) {
    console.error('Error: Bundle path is required');
    console.error('Usage: node server.js [port] <bundle-path>');
    process.exit(1);
  }

  try {
    const server = await Server.create(port, bundlePath);

    process.on('SIGINT', () => {
      server.close();
      process.exit(0);
    });
  } catch (error) {
    console.error('Failed to start server:', error);
    process.exit(1);
  }
}

main().catch(console.error);
